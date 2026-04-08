# SPEC-12 TDD Checklist

> テストファースト。各項目は **Red（失敗するテストを書く）→ Green（最小実装で通す）→ Refactor** の順で進める。
> テストコードは実装コードより先に commit する。

## Layer 1: Section parser (`sections.rs`)

- [ ] **RED-01**: `<!-- artifact:spec BEGIN --> foo <!-- artifact:spec END -->` から `("spec", "foo")` を抽出
- [ ] **RED-02**: 複数セクション（spec + tasks + plan）を順序付きで抽出
- [ ] **RED-03**: BEGIN はあるが END が無い → `ParseError::UnterminatedSection`
- [ ] **RED-04**: END のみ → `ParseError::UnmatchedEnd`
- [ ] **RED-05**: 同名セクションが本文内に 2 回出現 → `ParseError::DuplicateSection`
- [ ] **RED-06**: セクション名に `/` を含む contract セクション（`artifact:contract/api.yaml`）を正しく扱う
- [ ] **RED-07**: 先頭・末尾の改行を剥がした内容が返される

## Layer 2: SpecBody parse/render (`body.rs`)

- [ ] **RED-10**: body 先頭の `<!-- gwt-spec id=2001 version=1 -->` を `SpecMeta` に載せる
- [ ] **RED-11**: `<!-- sections: spec=body, plan=comment:111 -->` index マップを `SpecMeta.sections_index` に載せる
- [ ] **RED-12**: body と comments 配列を組み合わせて `SpecBody::parse` すると全セクションが取り出せる
- [ ] **RED-13**: `render` して再度 `parse` しても `SpecBody` が等価（round-trip property）
- [ ] **RED-14**: `SpecBody::splice(Tasks, new_content, &mut routing)` は Tasks 以外のセクションを 1 バイトも変更しない
- [ ] **RED-15**: 存在しないセクションへの splice は新規追加として動作する
- [ ] **RED-16**: index マップが壊れた body → `ParseError::BrokenIndex`

## Layer 3: Routing (`routing.rs`)

- [ ] **RED-20**: 15 KiB のセクションは `Body` にとどまる
- [ ] **RED-21**: 16 KiB + 1 byte のセクションは `Comment(None)` に昇格
- [ ] **RED-22**: body 内セクション合計が 60 KiB を超えるとき、最も大きい body セクションが comment に降格
- [ ] **RED-23**: 降格ルールは「spec と tasks は最後まで body を優先」する
- [ ] **RED-24**: routing 決定は純関数（副作用なし）で `SpecBody` と `sections_index` のみを入力とする

## Layer 4: IssueClient contract (`client.rs` + `fake.rs`)

- [ ] **RED-30**: `fetch(n, None)` が初回フェッチで `Updated` を返す
- [ ] **RED-31**: `fetch(n, Some(updated_at))` で `updatedAt` が同じなら `NotModified`
- [ ] **RED-32**: `fetch` は GraphQL `{ body, updatedAt, comments.nodes[] }` を返す fake を満たす
- [ ] **RED-33**: `patch_body` は 1 回の REST PATCH 相当の fake メソッドを呼ぶ
- [ ] **RED-34**: `patch_comment` は対応する comment ID に対して 1 回呼ぶ
- [ ] **RED-35**: `create_issue` → 返却された issue number を呼び出し側に返す
- [ ] **RED-36**: `list_spec_issues` はフェーズフィルタに応じて結果を返す（GraphQL 1 call 相当）

## Layer 5: HTTP client real implementation (`client/http.rs`)

- [ ] **RED-40**: `wiremock` でモックした GraphQL エンドポイントに対して fetch が通る
- [ ] **RED-41**: 認証ヘッダ `Authorization: Bearer <token>` が付与される
- [ ] **RED-42**: REST PATCH 時の `Content-Type: application/json` と body のペイロード
- [ ] **RED-43**: 422 (文字数上限) を受けて `ApiError::BodyTooLarge` に変換
- [ ] **RED-44**: 403 (rate limit) を受けて `ApiError::RateLimited { retry_after }` に変換
- [ ] **RED-45**: ネットワーク失敗を `ApiError::Network` に変換

## Layer 6: Cache (`cache.rs`)

- [ ] **RED-50**: `write_all(2001, &spec_body)` は tmp → rename で atomic に書き出す
- [ ] **RED-51**: 書き込み中に SIGINT が来ても既存 cache は破壊されない（tmp のみ残る）
- [ ] **RED-52**: flock 取得中は他プロセスが待機する
- [ ] **RED-53**: `history_rotate` が直近 3 世代を保持し、4 世代目を削除
- [ ] **RED-54**: `load_section(2001, Tasks)` が `sections/tasks.md` を返す
- [ ] **RED-55**: cache が存在しないときの `load` は `CacheMiss`

## Layer 7: spec_ops integration (`spec_ops.rs`)

- [ ] **RED-60**: `read_section(2001, Tasks)` — fake client が `NotModified` を返す → cache から読む
- [ ] **RED-61**: `read_section(2001, Tasks)` — fake client が `Updated` を返す → cache が書き換わる
- [ ] **RED-62**: `write_section(2001, Tasks, new)` — tasks が body 配置 → 呼び出しは `patch_body` 1 回のみ
- [ ] **RED-63**: `write_section(2001, Plan, big_new)` — plan が body 配置だが 17 KiB になる → 昇格 → `patch_comment` or `create_comment` + `patch_body`（計 2 呼び出し）
- [ ] **RED-64**: `write_section` 失敗時、cache は一切変更されない
- [ ] **RED-65**: `create_spec` は `create_issue → create_comment × N → patch_body` の順でコールされる
- [ ] **RED-66**: `create_spec` の初期 labels に `gwt-spec` と `phase/draft` が含まれる

## Layer 8: CLI dispatch (`gwt-tui/src/main.rs` + `cli/*`)

- [ ] **RED-70**: `gwt` （引数無し）→ TUI 起動パスに入る
- [ ] **RED-71**: `gwt issue spec 2001` → CLI ディスパッチ、`read_all_sections` を呼ぶ
- [ ] **RED-72**: `gwt issue spec 2001 --section tasks` → tasks のみ stdout
- [ ] **RED-73**: `gwt issue spec 2001 --section nonexistent` → 非ゼロ終了、stderr に可用セクション一覧
- [ ] **RED-74**: `gwt issue spec 2001 --edit tasks -f new.md` → `write_section` を 1 回呼ぶ
- [ ] **RED-75**: `-f -` で stdin から読む
- [ ] **RED-76**: `gwt issue spec list --phase=implementation` → `list_spec_issues` が該当 filter で呼ばれる
- [ ] **RED-77**: `gwt issue spec create --title T -f body.md` → `create_spec` が呼ばれ、作成後の issue number が stdout

## Layer 9: Migration (`migration.rs`)

- [ ] **RED-80**: `plan("specs/")` は 11 件の SPEC について `PlanEntry { old_id, title, predicted_sections, predicted_api_calls }` を返す
- [ ] **RED-81**: `plan` は実際の API コールを行わない（fake client で assert）
- [ ] **RED-82**: `execute` は 1 SPEC あたり `create_issue` 1 + `create_comment` N + `patch_body` 1 の順で呼ぶ
- [ ] **RED-83**: `execute` が途中で `ApiError::RateLimited` を受けたら 3 回までリトライ
- [ ] **RED-84**: `execute` 完了後、`migration-report.json` に全件の old ↔ new マッピングが書かれる
- [ ] **RED-85**: `execute` の冪等性: 既に `migration-report.json` に成功済み SPEC があれば再実行時にスキップ
- [ ] **RED-86**: `rewrite_docs` は fixture ディレクトリ内の `specs/SPEC-3` 参照を `#2003` に置換、`specs/SPEC-3/plan.md` のようなパス参照は警告を残す
- [ ] **RED-87**: `rewrite_docs` は git 履歴を書き換えない（working tree のみ）
- [ ] **RED-88**: `rollback` は成功済み Issue を `[ABANDONED]` 付きタイトルに書き換え、状態を closed にする

## Layer 10: E2E integration

- [ ] **RED-90**: テンポラリ cache + fake client で「新規 SPEC 作成 → tasks 編集 → 一覧取得」を通す
- [ ] **RED-91**: テンポラリ cache + wiremock で実 HTTP レイヤーを介した E2E
- [ ] **RED-92**: `migrate-specs --dry-run` を実 fixture `specs/` で実行、プレビュー出力をゴールデンファイル比較

## Layer 11: Search / startup sync

- [ ] **RED-100**: `gwt-search` の indexer が `~/.gwt/cache/issues/` を watch している
- [ ] **RED-101**: chunk の `type` メタデータが `spec` / `issue` で分離されている（ラベルに基づく）
- [ ] **RED-102**: `gwt` 起動時の同期タスクが `list_spec_issues` を 1 回だけ呼ぶ
- [ ] **RED-103**: 差分のある SPEC のみ `fetch` される（未更新はスキップ）
- [ ] **RED-104**: ネットワーク未接続のときは cache ベースで起動し、警告ログが出る

## Layer 12: TUI SPEC view

- [ ] **RED-110**: SPEC 一覧画面のスナップショットテスト（cache に 3 SPEC が存在する前提）
- [ ] **RED-111**: セクション切替 UI のスナップショットテスト
- [ ] **RED-112**: フェーズ変更操作が `set_labels` を呼ぶ（fake client で assert）

## Quality gates (pre-completion)

- [ ] `cargo test -p gwt-core -p gwt-tui -p gwt-github` 全緑
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 全緑
- [ ] `cargo fmt --check` パス
- [ ] `bunx commitlint --from origin/develop --to HEAD` パス
- [ ] 手動 E2E: テスト用 GitHub リポジトリに対して `migrate-specs --execute` → `--rollback` が正しく動作
- [ ] 本 SPEC 自身 (SPEC-12) がマイグレーション対象に含まれ、Issue 化されて `specs/SPEC-12/` が削除される

## Acceptance scenario coverage

| User Story | Acceptance | Test layer |
|---|---|---|
| US-1 | 未更新 → cache 返却 | Layer 7 RED-60 |
| US-1 | 初回 → GraphQL 1 call + cache 書き込み | Layer 7 RED-61 |
| US-1 | list は 1 call | Layer 4 RED-36, Layer 5 RED-40 |
| US-2 | 単一セクション出力 | Layer 8 RED-72 |
| US-2 | 存在しないセクション | Layer 8 RED-73 |
| US-3 | body→body 書き込み 1 call | Layer 7 RED-62 |
| US-3 | body→comment 昇格 2 call | Layer 7 RED-63 |
| US-3 | 失敗時 cache 不変 | Layer 7 RED-64 |
| US-4 | 16 KiB 超で昇格 | Layer 3 RED-21, Layer 7 RED-63 |
| US-4 | 60 KiB 超の降格 | Layer 3 RED-22 |
| US-4 | 422 の自動リトライ | Layer 5 RED-43, Layer 9 RED-83 |
| US-5 | dry-run プレビュー | Layer 9 RED-80, E2E RED-92 |
| US-5 | 冪等性 | Layer 9 RED-85 |
| US-5 | ドキュメント置換 | Layer 9 RED-86 |
| US-5 | rollback | Layer 9 RED-88 |
| US-6 | スキル更新 | tasks.md T-070〜T-079（コードテストではなくレビュー確認） |
| US-7 | 起動時 1 GraphQL | Layer 11 RED-102 |
| US-7 | 差分のみ fetch | Layer 11 RED-103 |
| US-8 | 一覧描画 | Layer 12 RED-110 |
| US-8 | セクション切替 | Layer 12 RED-111 |
| US-8 | フェーズ変更 | Layer 12 RED-112 |
