### 背景

`gwt-pr-check` の運用中に、直近の merged PR (`#1524`) が存在し、`origin/<head>..HEAD = 0` で新規コミットが無いにもかかわらず、LLM が `merge_commit..HEAD = 10` を根拠に `CREATE PR` と誤報告した。

今回の誤報告は LLM 実行の問題だが、現行ワークフローでは `gwt-pr-check` の仕様判断を LLM が手計算で再現しており、merge commit が branch `HEAD` の祖先でないケースで fallback を誤る余地がある。

この改善では、`gwt-pr-check` / `gwt-pr` の post-merge 判定を deterministic にし、LLM が同じ誤判定を返さないようにする。

### ユーザーシナリオとテスト（受け入れシナリオ）

- US1 (P0): 直近 PR が merged で、`origin/<head>..HEAD = 0` の場合、`gwt-pr-check` は `NO ACTION` を返す。
- US2 (P0): merge commit が `HEAD` の祖先でない場合でも、`origin/<head>..HEAD > 0` なら `CREATE PR` を返し、`0` なら `NO ACTION` を返す。
- US3 (P0): `gwt-pr` は `gwt-pr-check` と同じ判定ロジックを使い、`gwt-pr-check` と矛盾する新規 PR 作成をしない。
- US4 (P1): 判定不能時は曖昧な説明ではなく、`MANUAL CHECK` と理由を返す。

### 機能要件

- FR-001: `gwt-pr-check` の post-merge commit 判定は、merge commit の祖先判定を必ず行わなければならない。
- FR-002: merge commit が `HEAD` の祖先でない場合、`origin/<head>..HEAD` を第一 fallback として使用しなければならない。
- FR-003: `origin/<head>..HEAD = 0` の場合、`ALL_MERGED_NO_NEW_COMMITS` / `NO ACTION` を返さなければならない。
- FR-004: `gwt-pr` は `gwt-pr-check` と同一の deterministic 判定ロジックを使わなければならない。
- FR-005: LLM 向け skill / command 文面は、「merge commit が祖先でない場合は `origin/<head>..HEAD` を優先する」ことを明示しなければならない。
- FR-006: 判定不能時は `CHECK_FAILED` / `MANUAL_CHECK` を返し、推測で `CREATE PR` を返してはならない。

### 非機能要件

- NFR-001: PR 状態判定は、同じ repo/head/base 状態に対して常に同じ結果を返すこと。
- NFR-002: post-merge 判定は自動テストで回帰検知できること。

### 成功基準

- SC-001: merged PR 後に新規コミットが無い branch で `gwt-pr-check` が `NO ACTION` を返す。
- SC-002: `gwt-pr` が同条件で新規 PR を作成しない。
- SC-003: merge commit 非祖先ケースの fallback が自動テストで保証される。
