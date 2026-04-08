# SPEC-12 Implementation Plan

## Summary

本プランは SPEC 管理を GitHub Issue 基盤へ戻すための実装設計である。過去の「第 2 期」実装が抱えていた API コール爆発・トークン爆発の 2 つの失敗原因を、**GraphQL 1 回取得 + ETag/updatedAt 差分 + セクション粒度のローカルキャッシュ + ハイブリッド (本文/コメント) ストレージ** の 4 点で同時に解決する。CLI は既存 `gwt` バイナリへの引数駆動サブコマンドとして同梱し、他プロジェクトでの追加配信を不要にする。

## Technical Context

### 影響範囲

| 対象 | 種別 | 内容 |
|---|---|---|
| `crates/gwt-github`（新規） | Rust crate | ETag/GraphQL/REST/cache/section-parser の実装 |
| `crates/gwt-core` | Rust | 設定読み込み、`gh auth token` 呼び出し、既存 SPEC 型の廃止 |
| `crates/gwt-tui` | Rust | `main.rs` に argv ディスパッチ、SPEC 表示ロジックを cache ベースへ |
| `.claude/skills/gwt-spec-design/` | スキル | CLI 呼び出しへインプレース更新 |
| `.claude/skills/gwt-spec-plan/` | スキル | 同上 |
| `.claude/skills/gwt-spec-build/` | スキル | 同上 |
| `.claude/skills/gwt-arch-review/` | スキル | 同上 |
| `.claude/skills/gwt-issue/` | スキル | `gwt issue spec` への委譲を追加 |
| `.claude/skills/gwt-search/` | スキル | インデックス対象を `~/.gwt/cache/issues/` に変更 |
| `.claude/skills/gwt-spec-to-issue-migration/` | スキル | 廃止 → 新マイグレーションのドキュメントへ |
| `.claude/scripts/spec_artifact.py` | スクリプト | 削除 |
| `specs/SPEC-1..11/` | データ | マイグレーション後に `git rm -rf` |
| `README.md` / `README.ja.md` | ドキュメント | SPEC 参照を `#<issue>` へ置換 |
| `CLAUDE.md` / `AGENTS.md` | ドキュメント | ワークフロー説明と `specs/SPEC-{N}/` 言及の書き換え |
| `.gitignore` | 設定 | `specs/` を追加（完了後） |
| `~/.gwt/cache/issues/` | ランタイム | 新規作成、Issue キャッシュの正本置き場 |

### 前提と制約

- GitHub REST API: Issue 本文 65,536 文字、コメント本文 65,536 文字、各リポジトリあたり `secondary rate limit` が存在する。
- GitHub GraphQL API: 1 時間 5,000 ポイント、1 Issue + 50 コメント取得は 1〜2 ポイント想定。
- `gh auth token` は既にインストール済みである前提（`gh auth status` で確認）。
- `crates/gwt-core` は現行で Git 操作・PTY・設定を抱えており、`gwt-github` はこの下層 crate としてリンクする。
- Rust 2021 edition / stable toolchain を維持する。

### 仮定（明示）

- 1 SPEC あたり平均 6 アーティファクト、コメント数は 50 未満。
- マイグレーション対象は常に `specs/SPEC-*/` の 11 件のみ（隠しディレクトリは対象外）。
- `gh auth` が使用不可の環境（CI 等）では CLI は明示エラーで終了する（cache のみの read-only モードは提供しない）。

## Constitution Check

### 必須ルール照合（`.gwt/memory/constitution.md` に相当する AGENTS.md の原則）

| ルール | 遵守状況 | 備考 |
|---|---|---|
| Plan Mode Default | ✅ | 本 plan.md 自体が Plan。実装中に前提が崩れれば更新する。 |
| Verification Before Done | ✅ | タスク毎に `cargo test` / `cargo clippy` / 手動 E2E を定義。 |
| Subagent Strategy | ✅ | `gwt-github` crate 実装 / skill 更新 / migration script 実装を並列サブエージェントで分割できる構造。 |
| Demand Elegance | ✅ | ハイブリッド配分を 16 KiB 閾値単体で決める（追加ルール無し）。GraphQL 1 call + REST PATCH 1 call の 2 層のみに限定。 |
| Autonomous Bug Fixing | ✅ | 不具合再現手順は quickstart.md と tdd.md の Red-Green 基準で自律的に特定可能。 |
| 設計・実装はシンプルに | ✅ | 新規 crate は 1 つ、既存 crate の肥大化は避ける。スキルはインプレース更新で新規ファイルを最小化。 |
| 変更は外科的に | ⚠️ | マイグレーションで `specs/` を消すのは大きな不可逆変更。ユーザー明示承認済みのため OK とする。 |
| 場当たり修正禁止 | ✅ | 旧実装のコメント分割は症状ではなく *根本原因*（API コール爆発）を特定し、GraphQL 1 call に置換する。 |
| ブランチ保護 | ✅ | 本 SPEC の作業は `feature/specs`（現在のブランチ）で完結、develop への直接 commit は無い。 |
| commitlint | ✅ | 実装段階の commit は `feat(spec)` / `refactor(skills)` / `chore(specs)` などの Conventional Commits を守る。マイグレーション完了 commit は `refactor(specs)!:` でメジャー相当を明示。 |
| 完了条件（エラーゼロ） | ✅ | `cargo test -p gwt-core -p gwt-tui -p gwt-github` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --check` を完了ゲートに組み込む。 |

### 複雑性トラッキング

- **大きな不可逆変更**: `git rm -rf specs/`。ユーザー承認済み。`migration-report.json` に旧 SPEC-N ↔ 新 Issue # の逆引きを残すことで、必要ならロールバック手順を実行可能にする。
- **TUI + CLI dual mode**: `gwt` バイナリの責務が増える。`main.rs` の argv 判定を最上位のみに留め、TUI 本体には argv を渡さない構造で責務境界を守る。
- **スキル同時改修**: 4 スキル + 1 スキル廃止 + 1 スクリプト削除を 1 PR にまとめる。マイグレーション実行前にすべてのスキルが両経路に対応するトランジション期間を短時間だけ設け、実行直後に旧経路を削除する。

### Plan Gates への回答

- **どのファイルが影響を受けるか**: 上記「影響範囲」表の通り。
- **どの憲法制約が適用されるか**: 外科的変更・シンプルさ・commitlint・完了条件・TDD。
- **どのリスク / 複雑性追加を許容し、なぜか**: `specs/` 廃止、gwt バイナリのデュアルモード、複数スキル同時改修。いずれも本 SPEC の目的（第 4 期移行）を達成するために必須で、中途半端な二重管理は過去の失敗を再現する。
- **受け入れシナリオはどう検証するか**: `tasks.md` の各 T-NNN 単位でテストを定義し、`tdd.md` の Red-Green-Refactor サイクルで実証する。E2E はテスト用 GitHub リポジトリに対する実 API コールで検証する（CI では録画済みフィクスチャ再生で OK）。

## Architecture Design

### コンポーネント

#### 1. `crates/gwt-github`（新規 crate）

責務: GitHub API 通信・キャッシュ・セクションパーサ・ハイブリッド配分を一手に担う。

モジュール:

- `client` — `IssueClient` trait と HTTPS 実装（reqwest blocking）
- `cache` — `~/.gwt/cache/issues/<n>/` の read/write・ファイルロック
- `body` — 本文 + コメントを結合したセクションビュー、`parse` / `render` / `splice(section, content)`
- `sections` — 境界マーカーの正規表現と、セクション名の enum
- `routing` — `body` か `comment:<cid>` かを決めるルーティング表と 16 KiB 昇格判定
- `spec_ops` — 高レベル API: `read_section`, `write_section`, `create_spec`, `list_specs`
- `migration` — `gwt issue migrate-specs` のロジック

外部依存: `reqwest` (blocking, rustls), `serde` / `serde_json`, `regex`, `fs2` (flock), `sha2` (etag fallback), `thiserror`.

#### 2. `crates/gwt-tui`（改修）

- `main.rs` に argv 判定を追加。`env::args().nth(1)` が `issue` / `spec` 等の既知サブコマンドなら CLI ディスパッチ、それ以外は従来通り TUI 起動。
- SPEC 表示画面は `gwt-github::spec_ops::read_section` を呼び出して cache から描画（API 呼び出しは行わない）。
- 起動時同期は `tokio::spawn` でバックグラウンド実行（TUI 初期化を待たせない）。

#### 3. CLI サブコマンド構造

```
gwt                                 # TUI (既存)
gwt issue spec <n>                  # 全セクション出力
gwt issue spec <n> --section <name> # 単一セクション出力
gwt issue spec <n> --edit <section> -f <file>
gwt issue spec list [--phase <p>] [--label <l>]
gwt issue spec create --title <t> -f <body_file>
gwt issue spec pull [--all | <n>...]
gwt issue spec repair <n>
gwt issue migrate-specs [--dry-run | --execute | --rollback]
```

### インターフェース契約

#### `IssueClient` trait

```rust
pub trait IssueClient {
    fn fetch(&self, n: IssueNumber, since: Option<UpdatedAt>) -> Result<FetchResult>;
    fn patch_body(&self, n: IssueNumber, new_body: &str) -> Result<IssueSnapshot>;
    fn patch_comment(&self, comment_id: CommentId, new_body: &str) -> Result<CommentSnapshot>;
    fn create_comment(&self, n: IssueNumber, body: &str) -> Result<CommentSnapshot>;
    fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Result<IssueSnapshot>;
    fn set_labels(&self, n: IssueNumber, labels: &[&str]) -> Result<()>;
    fn list_spec_issues(&self, filter: SpecListFilter) -> Result<Vec<SpecSummary>>;
}

pub enum FetchResult {
    NotModified,
    Updated(IssueSnapshot), // body + comments + updatedAt
}
```

GraphQL クエリは `list_spec_issues` と `fetch` の 2 種類のみ。書き込みはすべて REST。

#### `body` モジュール

```rust
pub struct SpecBody {
    pub meta: SpecMeta,
    pub sections: HashMap<SectionName, SectionContent>,
}

impl SpecBody {
    pub fn parse(body: &str, comments: &[Comment]) -> Result<Self>;
    pub fn render(&self) -> RenderResult;
    // RenderResult は本文文字列と、更新すべきコメント ID リストを返す。
    pub fn splice(&mut self, section: SectionName, content: &str, routing: &mut Routing);
}
```

### データモデル概要

詳細は `data-model.md` を参照。要点:

- `SpecMeta { id, title, labels, updated_at, sections_index }` が本文先頭の HTML コメントと `meta.json` に格納される。
- `SectionName` は enum: `Spec`, `Plan`, `Tasks`, `Research`, `DataModel`, `Quickstart`, `Contract(String)`。
- `SectionContent { location: Body | Comment(CommentId), raw_markdown: String, byte_size: usize }`。
- キャッシュレイアウト:

```text
~/.gwt/cache/issues/<n>/
├── body.md                   # 最新本文（タイトルは meta.json）
├── meta.json                 # SpecMeta のシリアライズ
├── sections/
│   ├── spec.md
│   ├── plan.md
│   ├── tasks.md
│   ├── research.md
│   ├── data-model.md
│   └── quickstart.md
├── comments/
│   ├── 3145926.md            # コメント ID 別の生 body
│   └── 3145927.md
└── history/
    ├── body.1.md             # 直近 3 世代
    ├── body.2.md
    └── body.3.md
```

### キーシーケンス

#### A. セクション読み取り（単一）

1. `gwt issue spec 2001 --section tasks` が CLI に入る
2. `spec_ops::read_section(2001, Tasks)` が呼ばれる
3. `cache::load_meta(2001)` → `meta.json` があれば `updated_at` 取得、無ければ初回扱い
4. `client.fetch(2001, Some(updated_at))` → GraphQL 1 呼び出し
5. `FetchResult::NotModified` なら `cache::load_section(2001, Tasks)` を stdout
6. `FetchResult::Updated(snapshot)` なら `SpecBody::parse` → `cache::write_all(2001, &spec_body)` → セクションを stdout
7. ネットワーク失敗時は cache を返し、stderr に警告

#### B. セクション書き込み

1. `gwt issue spec 2001 --edit tasks -f new_tasks.md`
2. `cache::lock(2001)` で flock
3. 読み取りと同じフローで最新の `SpecBody` を取得（書き込み前は常にフェッチ）
4. `spec_body.splice(Tasks, &new_content, &mut routing)`
5. `routing` が `Body` のままなら `client.patch_body(n, new_body)` 1 call
6. `routing` が `Comment(cid)` に変わったなら:
   - 既存 body の tasks セクションをプレースホルダに差し替え
   - 既存コメント存在時は `patch_comment` 1 call、無ければ `create_comment` 1 call
   - 本文の index マップ更新を `patch_body` 1 call（＝計 2 call）
7. 成功なら `cache::write_all`、flock 解除
8. 失敗時は cache 変更せず、flock 解除、非ゼロ終了

#### C. 新規 SPEC 作成

1. `gwt issue spec create --title "..." -f draft_body.md`
2. draft_body.md のセクションを `SpecBody::parse` （comments=[] 前提）
3. 16 KiB を超えるセクションは `routing` 上で `Comment` マークするが、実際の create は下記順で実行:
   - `create_issue(title, initial_body_with_body_sections_only, labels=[gwt-spec, phase/draft])` → Issue 番号取得
   - comment 側のセクションを順に `create_comment`
   - 最終的な `sections_index` を反映した本文に `patch_body`
4. cache 初期化

#### D. `migrate-specs --execute`

1. `specs/` 走査、11 件を順次処理
2. 各 SPEC で以下を実行:
   - `spec.md` / `tasks.md` を body 候補、`plan.md` / `research.md` / `data-model.md` / `quickstart.md` / `contracts/*` を comment 候補
   - 16 KiB を超える body 候補は comment に降格
   - `create_issue` 1 call → `create_comment` × N call → `patch_body`（index 更新）1 call
   - `migration-report.json` に逐次追記
   - 次の SPEC との間に 2 秒 sleep
3. 全成功で `git rm -rf specs/`、`.claude/scripts/spec_artifact.py` 削除、ドキュメント置換、`.gitignore` 追加
4. 失敗時は sleep を挟みつつリトライ 3 回、なお失敗ならその SPEC をスキップして次へ。最後に未成功 SPEC 一覧を stderr 表示。

## Project Structure

```text
crates/
├── gwt-core/           # 既存
├── gwt-github/         # 新規
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs
│       ├── cache.rs
│       ├── body.rs
│       ├── sections.rs
│       ├── routing.rs
│       ├── spec_ops.rs
│       └── migration.rs
├── gwt-tui/            # 既存 + argv ディスパッチ追加
└── ...
.claude/
├── skills/
│   ├── gwt-spec-design/       (更新)
│   ├── gwt-spec-plan/         (更新)
│   ├── gwt-spec-build/        (更新)
│   ├── gwt-arch-review/       (更新)
│   ├── gwt-issue/             (更新)
│   ├── gwt-search/            (更新)
│   └── gwt-spec-to-issue-migration/  (廃止 → README のみ)
└── scripts/
    └── spec_artifact.py       (削除)
specs/                          (マイグレーション後に削除)
```

## Complexity Tracking

- ハイブリッド配分の 16 KiB 閾値は単一の定数 `ROUTING_PROMOTE_THRESHOLD_BYTES` に集約する（後の調整を容易にする）。
- `SpecBody::splice` は副作用を持たない純関数に保ち、routing 決定と Rust 側のバリデーションを分離する。
- マイグレーションスクリプトは Rust 実装に一本化し、Node.js / Python の旧ファイルは削除する。

## Phased Implementation

### Phase 1: 基盤 crate 構築

1. `crates/gwt-github` を scaffold、`Cargo.toml` 登録
2. `sections` モジュール（正規表現・境界パーサ・enum）を TDD
3. `body::parse` / `render` / `splice` を TDD（純関数テスト）
4. `routing` の 16 KiB 昇格ロジックを TDD

### Phase 2: クライアント層と cache

5. `client` の `IssueClient` trait 定義、fake impl によるテスト
6. HTTPS 実装（reqwest blocking + rustls）、契約テスト
7. `cache` の atomic write / flock / history ローテーション
8. `spec_ops::read_section` / `write_section` を統合テスト（fake client）

### Phase 3: CLI ディスパッチ

9. `gwt-tui` の `main.rs` に argv 判定を追加、サブコマンドディスパッチ
10. `gwt issue spec <n>` / `--section` / `--edit` / `list` / `create` / `pull` / `repair` の実装
11. 統合テスト: 一時 cache ディレクトリ + fake client で E2E

### Phase 4: マイグレーション実装

12. `migration::plan` (dry-run) の実装と単体テスト（fixture の `specs/` ディレクトリに対する動作）
13. `migration::execute` の実装（fake client）
14. ドキュメント置換ロジック（README / CLAUDE.md / AGENTS.md）
15. `--rollback` の実装

### Phase 5: スキル更新

16. `gwt-spec-design` を `gwt issue spec create` 呼び出しに書き換え
17. `gwt-spec-plan` を `gwt issue spec <n> --section` / `--edit` 呼び出しに書き換え
18. `gwt-spec-build` の tasks 更新を `--edit tasks` 経由へ
19. `gwt-arch-review` の横断分析を `gwt issue spec list` 経由へ
20. `gwt-issue` に SPEC 向け委譲を追加
21. `gwt-search` のインデックス対象を `~/.gwt/cache/issues/` へ、type 分離
22. `gwt-spec-to-issue-migration` の SKILL.md を廃止通知に置き換え
23. `.claude/scripts/spec_artifact.py` を削除

### Phase 6: 実 API マイグレーション実行

24. テスト用 GitHub リポジトリで `migrate-specs --dry-run` → `--execute` の予行演習
25. 本リポジトリで `--dry-run` 実行
26. レビュー後 `--execute` 実行、`specs/` 削除コミット
27. README / CLAUDE.md / AGENTS.md の最終調整

### Phase 7: TUI 統合

28. TUI の SPEC 一覧画面を cache 駆動に切替
29. SPEC 詳細画面・セクション切替 UI の実装
30. 起動時バックグラウンド同期の実装

### Phase 8: 完了ゲート

31. `cargo test -p gwt-core -p gwt-tui -p gwt-github`
32. `cargo clippy --all-targets --all-features -- -D warnings`
33. `cargo fmt --check`
34. 最終 E2E: 新規 SPEC 作成 → セクション編集 → tasks チェック → 一覧検索 → close
35. PR 作成（develop 向け）

## Rollback Plan

- `gwt issue migrate-specs --rollback` により作成済み Issue に `[ABANDONED]` 接頭辞付きで close、ローカル `specs/` を `git checkout HEAD~<N> -- specs/` で復旧。
- Rust 実装側は crate 追加 / 既存 crate への改修のみで、commit 粒度を細かく分ければ個別 revert が可能。
- スキル更新は 1 PR にまとめるため、PR 丸ごと revert で旧状態に戻る。
