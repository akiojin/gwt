# SPEC-12: SPEC Management via GitHub Issues — Token-Minimized Hybrid Storage

## Background

gwt が管理する SPEC は過去 3 度にわたり保管場所を変えてきた:

1. **ローカルファイル**（初期）— `specs/SPEC-{N}/*.md`
2. **GitHub Issue**（第 2 期）— `gwt-spec` ラベル付き Issue + 「1 アーティファクト = 1 コメント」形式
3. **ローカルファイル**（現行）— `specs/SPEC-{N}/*.md` に再度戻された

第 2 期から第 3 期への移行理由は `gh` CLI 経由の API 呼び出しが頻繁にレートリミットに到達したことだった。その根本原因は当時の仕様上、1 SPEC の読み込みに `gh issue view` + `gh issue comment list` + 各コメント fetch が必要で、1 SPEC あたり 7〜9 API call、11 SPEC で 80 call 規模の突発トラフィックが発生していたことにある（`.claude/skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.mjs` でも `RATE_LIMIT_BATCH=10` / `sleep 3s` / 5 回リトライの対症療法が確認できる）。

本 SPEC は第 4 期として再び GitHub Issue を SPEC の唯一の真実（SOT）とする。ただし以下を満たすことを目的とする:

- 1 SPEC の読み取りを **GraphQL 1 回の API コール** に圧縮する
- エージェントが SPEC を参照するときの **コンテキスト注入トークンをセクション粒度に限定** する
- Issue 本文 64 KiB 上限に対して **ハイブリッド・ストレージ**（本文＋コメント）で安全に収まる
- ローカル `specs/SPEC-*/` ディレクトリは **完全に廃止** し、作業ツリーの汚染をゼロにする
- CLI は `gwt` バイナリのサブコマンドとして配信され、他プロジェクトでも追加インストール不要

## User Stories

### US-1: 単一 API コールでの SPEC 読み取り (P0)

As a gwt エージェント、I want 1 SPEC の全アーティファクト（spec/plan/tasks/research/data-model/quickstart/contracts）を 1 回の GitHub API コールで取得できること, so that API レートリミットが実運用で発生しない。

**Acceptance Scenarios:**

1. Given 既に `gwt issue spec pull 2001` でキャッシュされた SPEC、when `gwt issue spec 2001` を実行、then 更新検知のために GraphQL を 1 回実行し、未更新であればキャッシュを使用する。
2. Given 未キャッシュの SPEC、when `gwt issue spec 2001` を実行、then GraphQL 1 回で Issue 本文と全コメントを取得し、セクション分割してキャッシュに書き出す。
3. Given 11 個の SPEC、when `gwt issue spec list --phase=implementation` を実行、then GraphQL 1 回でフィルタ済み一覧を返す。

### US-2: セクション粒度の読み取り (P0)

As a gwt エージェント、I want SPEC の特定セクションだけを読み取れること, so that エージェントのコンテキストに不要なアーティファクトが注入されずトークン消費が最小化される。

**Acceptance Scenarios:**

1. Given SPEC 2001 がキャッシュ済み、when `gwt issue spec 2001 --section tasks` を実行、then tasks セクションのみを stdout に出力する（body/comment の配置は透過的）。
2. Given エージェントが spec と tasks のみ必要、when それぞれ `--section` 指定で取得、then plan/research/data-model/quickstart は一切読み込まれず、コンテキストサイズは要求セクションの合計サイズに比例する。
3. Given 存在しないセクション名、when 指定、then 非ゼロ終了コードと、利用可能なセクション一覧を stderr に出力する。

### US-3: セクション単位の書き込み (P0)

As a gwt エージェント、I want SPEC の特定セクションだけを書き換えられること, so that tasks チェック 1 つのために SPEC 全体を PATCH する事故が起きない。

**Acceptance Scenarios:**

1. Given SPEC 2001、when `gwt issue spec 2001 --edit tasks -f new_tasks.md` を実行、then その他セクション（spec/plan/research/...）は 1 バイトも変更されず、tasks のセクションだけが GitHub 上で更新される。
2. Given tasks がハイブリッド配置の本文側、when `--edit tasks` を実行、then REST PATCH `/repos/:o/:r/issues/:n` が 1 回だけ呼ばれる。
3. Given plan がコメント側、when `--edit plan` を実行、then 対応するコメント ID に対して REST PATCH `/repos/:o/:r/issues/comments/:cid` が 1 回だけ呼ばれ、本文側は触らない。
4. Given 書き込み後、when キャッシュを確認、then ローカルのセクション分割ファイルと index マップが atomic に更新されている。

### US-4: 本文 64 KiB 上限の自動回避（ハイブリッド昇格）(P0)

As a SPEC 作成者、I want SPEC が大きくなっても本文の文字数上限で壊れないこと, so that 長大な plan や research を含む SPEC も問題なく運用できる。

**Acceptance Scenarios:**

1. Given 書き込むセクションのサイズが 16 KiB 以下、when そのセクションが本文配置、then 本文のまま維持される。
2. Given 書き込むセクションのサイズが 16 KiB を超える、when そのセクションが現在本文配置、then 自動的にコメントに昇格され、本文にはプレースホルダと index マップ上の `comment:<id>` 参照が記録される。
3. Given Issue 本文全体が 60 KiB を超える書き込み、when 実行、then 警告と自動昇格（複数セクションの一括退避）が発動し、最終的に本文は 60 KiB 未満に収まる。
4. Given GitHub API が 65536 文字上限エラー（422）を返却、then CLI は失敗ではなく昇格リトライを 1 回試行する。

### US-5: 既存 11 SPEC の一括マイグレーション (P0)

As a プロジェクトオーナー、I want 現行 `specs/SPEC-1`〜`SPEC-11` を 1 回のコマンドで Issue 化し、その後ローカル SPEC ディレクトリを完全廃止できること, so that 二重管理の期間をゼロにできる。

**Acceptance Scenarios:**

1. Given `specs/SPEC-*/` が存在、when `gwt issue migrate-specs --dry-run` を実行、then 作成予定の Issue 件数・本文/コメント配分・推定 API コール数を出力し、何も変更しない。
2. Given dry-run で問題なし、when `gwt issue migrate-specs --execute` を実行、then 各 SPEC を順次 Issue 化し、1 SPEC ≤ 6 API call（create + 必要コメント数）を守る。
3. Given 全 SPEC の Issue 化に成功、when マイグレーション完了、then `specs/` ディレクトリが `git rm -rf` され、`migration-report.json` に旧 SPEC-N ↔ 新 Issue # のマッピングが残る。
4. Given マイグレーション途中で失敗、when 次回 `--execute` を実行、then 作成済み Issue をスキップし、未作成の SPEC のみ再試行する（冪等性）。
5. Given マイグレーション完了後、when README.md / CLAUDE.md / AGENTS.md / tasks.md 内の `specs/SPEC-N` 参照、then すべて `#<issue-number>` に自動置換されている。

### US-6: 既存スキル群のインプレース更新 (P0)

As a gwt エージェント、I want `gwt-spec-design` / `gwt-spec-plan` / `gwt-spec-build` / `gwt-arch-review` が現行のコマンド名のまま Issue 経路で動作すること, so that ユーザーの `/gwt:*` 呼び出し習慣を壊さず移行できる。

**Acceptance Scenarios:**

1. Given ユーザーが `/gwt:gwt-spec-design` を実行、when スキルが新規 SPEC を作成、then `gwt issue spec create` が呼ばれて GitHub Issue が作成される（ローカルファイル作成は発生しない）。
2. Given 既存 SPEC 上で `/gwt:gwt-spec-plan 2001` を実行、when プラン策定を進める、then `gwt issue spec 2001 --section spec` で spec を読み、`--edit plan/tasks/research/data-model/quickstart` で成果物を書き込む。
3. Given `/gwt:gwt-spec-build 2001` を実行、when タスク完了時に tasks を更新、then `gwt issue spec 2001 --edit tasks` 経由でチェックボックスが更新される。
4. スキルが従来使っていた `.claude/scripts/spec_artifact.py` は削除されている。

### US-7: gwt 起動時の自動インデックス更新 (P1)

As a gwt 利用者、I want `gwt` 起動時に最新の SPEC 一覧が自動で取り込まれ、`gwt-search --specs` がすぐに機能すること, so that 他のメンバーが GitHub で作成した SPEC を意識せず検索できる。

**Acceptance Scenarios:**

1. Given `gwt` を起動、when 起動初期化フェーズ、then 1 GraphQL 呼び出しで `gwt-spec` ラベル Issue の更新差分を取得し、必要なものだけ cache を更新する。
2. Given ChromaDB、when cache 更新が発生、then watcher が差分をインデックスに反映する。
3. Given 起動中、when ネットワーク未接続、then 既存 cache を使い警告をログに記録する（起動は失敗しない）。
4. 起動時以外の更新は `gwt issue spec pull --all` を手動で叩くことで行う（バックグラウンド同期は行わない）。

### US-8: TUI からの SPEC 操作 (P1)

As a gwt-tui 利用者、I want TUI から SPEC の一覧・詳細・ラベル変更ができること, so that CLI を意識せず GUI 的に運用できる。

**Acceptance Scenarios:**

1. Given TUI の Management レイヤー、when SPEC 一覧を開く、then cache から即座にレンダリングされる。
2. Given SPEC 詳細画面、when セクションを切り替える、then cache のセクションファイルを読み取り瞬時に表示する（API コール 0）。
3. Given フェーズ変更操作、when 実行、then REST PATCH でラベルを更新し、cache の meta.json も反映する。

## Edge Cases

- **Issue 本文が手動編集でセクションマーカーが壊れた場合** — `gwt issue spec <n> --repair` でバックアップから再構築。バックアップは直近 3 世代の本文を cache 内 `history/` に持つ。
- **コメントがユーザーにより手動削除された場合** — index マップの `comment:<id>` 参照が 404 になる → `--repair` で該当セクションを空として再初期化し、警告を表示。
- **同一 SPEC を複数エージェントが同時編集** — last-write-wins。`updated_at` 比較で事後検知し、競合発生時は後勝ちエージェントの stderr に警告。楽観ロックは初期バージョンでは導入しない。
- **コメント 50 件超の SPEC** — GraphQL ページングが必要。初期バージョンは 50 件上限を警告付きで強制、50 件超は `--repair` で圧縮提案。
- **GraphQL レート制限（1 時間 5000 ポイント）** — 1 SPEC あたり 1 ポイント換算で、通常使用では到達しない。バッチ `pull --all` は 200 SPEC 以下を想定。
- **ネットワーク未接続** — 読み取りは cache を返し 0 API コール。書き込みは明示エラー終了（キューイングしない）。
- **`gh auth` 未認証** — 最初の CLI 実行時にエラーメッセージ + `gh auth login` への誘導を表示。
- **Issue の転送・番号付け替え** — GitHub Issue は転送できないため考慮不要。ただしリポジトリ移動時は `gwt issue migrate-repo` のような別経路が必要（本 SPEC では対応しない）。
- **マイグレーション途中失敗** — `migration-report.json` にスナップショットを書き続ける。再実行時は既作成 Issue を検出してスキップ。
- **マイグレーション後の commit/PR 本文に残る旧 SPEC-N 参照** — git 履歴は書き換えず、README/CLAUDE.md/AGENTS.md/現行 tasks 等の *現在* 参照されるドキュメントのみ自動置換。過去履歴内の参照は古い ID として残す。

## Functional Requirements

### FR 群 A: ストレージと本文フォーマット

- **FR-001**: 1 SPEC = 1 GitHub Issue、`gwt-spec` ラベル必須、`phase/{draft|planning|implementation|review|done}` のいずれか 1 つを付与。
- **FR-002**: Issue 本文先頭に HTML コメントでメタヘッダ `<!-- gwt-spec id=<issue_number> version=1 -->` を持つ。
- **FR-003**: Issue 本文に `<!-- sections: ... -->` の index マップを持ち、各セクションが `body` または `comment:<comment_id>` にマップされる。
- **FR-004**: 各セクションの境界は HTML コメント `<!-- artifact:<name> BEGIN -->` / `<!-- artifact:<name> END -->` のペアで囲む（本文側・コメント側ともに）。
- **FR-005**: 配置の既定は `spec=body`、`tasks=body`、`plan=comment`、`research=comment`、`data-model=comment`、`quickstart=comment`、`contracts/*=comment`。
- **FR-006**: セクション書き込み時、そのセクション単独サイズが 16 KiB を超える場合は自動的に本文 → コメントへ昇格する。
- **FR-007**: Issue 本文全体が 60 KiB を超える書き込みは、最も大きい body 側セクションをコメント昇格させてリトライする。
- **FR-008**: `tasks` セクションの本文内表記は GitHub Task List チェックボックス `- [ ] T-001 ...` を標準とし、in-progress/blocked は行末 `[in-progress @agent-name]` のインラインタグで表現する。

### FR 群 B: CLI 層

- **FR-010**: `gwt` バイナリは引数無しで従来通り TUI を起動し、引数あり（`gwt issue ...` 等）ではサブコマンド CLI モードに入る。
- **FR-011**: `gwt issue spec <n>` は全セクションを stdout に出力する（本文とコメントをセクション順に並べた展開結果）。
- **FR-012**: `gwt issue spec <n> --section <name>` は指定セクションのみを出力する。存在しなければ非ゼロ終了し stderr に可用セクション一覧を出す。
- **FR-013**: `gwt issue spec <n> --edit <section> -f <file>` は指定セクションだけを書き換える。`-f -` で stdin 読み込みを許可する。
- **FR-014**: `gwt issue spec list [--phase=<p>] [--label=<l>]` は GraphQL 1 回で `gwt-spec` Issue を列挙する。
- **FR-015**: `gwt issue spec create --title <t> -f <body_file>` は新規 SPEC Issue を作成し、セクションを自動配分した上で `phase/draft` ラベルを付与する。
- **FR-016**: `gwt issue spec pull [--all | <n>...]` は cache を強制更新する。`--all` は `gwt-spec` ラベル全件を対象にし、差分がある Issue のみ更新する。
- **FR-017**: `gwt issue spec repair <n>` は cache / index マップを Issue の実状から再構築する。
- **FR-018**: `gwt issue migrate-specs [--dry-run | --execute] [--rollback]` は `specs/SPEC-*/` を Issue 化する。

### FR 群 C: キャッシュと API 層

- **FR-020**: キャッシュルートは `~/.gwt/cache/issues/` とし、1 Issue あたり `<n>/{body.md, meta.json, sections/*.md, comments/<cid>.md, history/}` の構造を持つ。
- **FR-021**: `meta.json` は `{ id, title, updated_at, etag (optional), labels, sections: { name -> body|comment:<cid> } }` を保持する。
- **FR-022**: 読み取りフローは GraphQL 1 呼び出しで `{ issue.body, issue.updatedAt, comments.nodes[] }` を取得し、`updatedAt` 比較で更新なしならキャッシュを返す。
- **FR-023**: 書き込みフローは (1) 最新取得 (2) 本文 or コメントの該当セクションを置換 (3) PATCH (4) cache atomic 更新、の 4 段階で行い、競合時は last-write-wins。
- **FR-024**: REST API は `https://api.github.com`、GraphQL API は `https://api.github.com/graphql` を使用し、認証は `gh auth token` で取得した PAT を使う。
- **FR-025**: 書き込みは REST PATCH のみ（本文 `/repos/:o/:r/issues/:n`、コメント `/repos/:o/:r/issues/comments/:cid`）。新規コメント作成は REST POST `/repos/:o/:r/issues/:n/comments`。
- **FR-026**: 同一 Issue に対する書き込みは CLI プロセス内でシリアライズする（キャッシュの Flock で保護）。

### FR 群 D: スキルとワークフロー統合・ユーザー言語

- **FR-029**: 本スキル（`gwt-spec-design` / `gwt-spec-plan` / `gwt-spec-build` / `gwt issue migrate-specs`）は、Issue タイトル・本文・コメント・自動挿入テキストを **実行時のユーザー言語**（LLM とユーザーが対話している言語）で生成する。スキル本体（SKILL.md・スクリプト・コード・コメント）は普遍化のため英語で記述してよいが、出力（ユーザー目線の成果物）はハードコードせずユーザー言語に従う。SPEC ファイル本文の翻訳が必要な場合も同原則に従う。


- **FR-030**: `gwt-spec-design` は `gwt issue spec create` を呼び出して新規 SPEC を作成し、ローカルファイルを作成しない。
- **FR-031**: `gwt-spec-plan` は `gwt issue spec <n> --section spec` で読み取り、`--edit plan/tasks/research/data-model/quickstart/contract/*` で書き込む。
- **FR-032**: `gwt-spec-build` は tasks のチェックを `gwt issue spec <n> --edit tasks` で反映する。
- **FR-033**: `gwt-arch-review` は `gwt issue spec list` と個別セクション取得を組み合わせて全 SPEC の横断分析を行う。
- **FR-034**: `gwt-search` は ChromaDB の対象ディレクトリを `~/.gwt/cache/issues/` に切り替え、各チャンクに `type=spec` または `type=issue` のメタデータを付与する。
- **FR-035**: `gwt-issue` スキルは汎用 Issue 用途を維持し、SPEC 固有の処理は `gwt issue spec` サブコマンドに委譲する。
- **FR-036**: `gwt-spec-to-issue-migration` スキルは廃止し、新 `gwt issue migrate-specs` のドキュメントとして再構成する。
- **FR-037**: `.claude/scripts/spec_artifact.py` は削除される。

### FR 群 E: マイグレーション

- **FR-040**: `gwt issue migrate-specs --dry-run` は作成予定 Issue 件数、推定 API コール総数、推定所要時間、本文/コメント配分プレビューを出力する。
- **FR-041**: `--execute` は各 SPEC について Issue 作成 → 必要数のコメント作成 → index マップを含む本文の最終 PATCH、を 1 SPEC あたり ≤ 6 API コールで実行する。バースト回避のため各 SPEC 間に 2 秒スリープを入れる。
- **FR-042**: マイグレーション中、各 SPEC の結果を `migration-report.json` に逐次追記する。再実行時は成功済みをスキップする（冪等）。
- **FR-043**: 全 SPEC 成功後、`git rm -rf specs/` / `.claude/scripts/spec_artifact.py` 削除 / `.gitignore` への `specs/` 追加 / README・CLAUDE.md・AGENTS.md 内の `specs/SPEC-N` 参照を `#<issue>` へ自動置換、を一括実行する。
- **FR-044**: `--rollback` は `migration-report.json` を元に、作成済み Issue を `[ABANDONED by migration rollback]` タイトル接頭辞付きでクローズし、ローカル `specs/` を git から復旧する。
- **FR-045**: マイグレーションは自身（SPEC-12）を含めて実行でき、SPEC-12 もマイグレーション対象の 1 つとして Issue 化される。

### FR 群 F: 起動時インデックス

- **FR-050**: `gwt` 起動時に `gwt-spec` ラベルの Issue リストを GraphQL 1 回で取得し、`updated_at` 差分があるものだけ `pull` して cache を更新する。
- **FR-051**: 起動時同期はバックグラウンドスレッドで実行され、TUI 起動自体はブロックしない。
- **FR-052**: ネットワーク未接続・認証失敗時は cache のみで起動し、警告を `~/.gwt/logs/` に記録する。
- **FR-053**: 起動時以外のバックグラウンド自動同期は行わない。明示的に `gwt issue spec pull --all` を呼んだときのみ更新する。

## Success Criteria

### SC-1: API コール削減（定量）

- 1 SPEC の全アーティファクト読み取り: **旧 7〜9 call → 新 1 GraphQL call**（≥ 85% 削減）
- 1 SPEC の tasks チェック 1 つ更新: **旧 1〜2 call → 新 1 REST PATCH call**（1 回あたりのペイロードも削減）
- 11 SPEC 分の再読取（更新無し想定）: **旧 ≥ 77 call → 新 1 GraphQL call**（updatedAt 差分のみ取得）
- 新規 6 アーティファクト SPEC の作成: **旧 7 call → 新 ≤ 6 call**

### SC-2: エージェントトークン削減（定量）

- `gwt-spec-build` 実行時の典型的な SPEC コンテキスト注入量: **旧 15k〜40k tok/SPEC → 新 2k〜6k tok/SPEC**（セクション読み取りに切替）
- `gwt-spec-plan` 実行時: **旧 40k 前後 → 新 8k 前後**（spec セクションのみ読み込み、plan/tasks は書き込み対象のみ）

### SC-3: 文字数上限回避

- 任意の大きさの SPEC（plan 40 KiB、research 20 KiB、data-model 20 KiB、quickstart 5 KiB を含むケース）が 422 エラー無しで保存できる。
- 上限に到達しそうなケースは自動昇格で回避される（ユーザー操作不要）。

### SC-4: ゼロダウンタイム移行

- `gwt issue migrate-specs --execute` 実行中から完了までの間、既存スキル呼び出しが壊れない（スキル内で旧ローカル経路と新 Issue 経路を切り替える feature flag を持つ）。
- 完了後、`specs/` ディレクトリが完全に削除され、どこかのドキュメントに古い参照が残っていないことを grep で確認できる。

### SC-5: 他プロジェクトでの再利用性

- `gwt` を別のプロジェクトに `cargo install` 等で導入した直後から、追加の Python スクリプトを配布することなく `gwt issue spec *` が動作する。
- 別プロジェクトでも同一フォーマットで SPEC を Issue 化できる（リポジトリ依存のパスや設定を CLI フラグで上書き可能）。

## Out of Scope

- GitHub Projects との自動連携（将来 SPEC で検討）
- GraphQL mutation による本文編集（REST PATCH で十分）
- 楽観ロック / OCC（将来 SPEC で検討）
- 複数リポジトリ横断の SPEC 管理
- 過去 commit メッセージや既にマージ済み PR 本文の書き換え（git 履歴は不変）
- Webhook 駆動のリアルタイム同期
- Issue コメントへの CI/bot 投稿との共存設計（汎用 `gwt-issue` 側で対応）

## Open Questions

現時点ですべてのクリティカルな判断は確定済み。実装フェーズで浮上した詳細は `plan.md` / `research.md` に追記する。
