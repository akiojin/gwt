> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

Cleanup の branch safety が同一 head branch に紐づく全 PR ではなく、実質的に latest PR 1 件の state を優先して判定している。そのため、同じ branch に `closed/unmerged` または `open` の PR が残っていても、後続の merged PR があると safe 扱いになり、ユーザーが未マージ PR 履歴を見落としたまま branch deletion に進めてしまう。

一方で `gwt-pr-check` / `gwt-pr` は `mergedAt == null` を未マージの source of truth として扱う運用になっており、gwt 本体の Cleanup 判定と意味論が不一致になっている。

この修正では、branch safety、latest PR 表示、post-merge commit 判定の責務を分離し、Cleanup では「同一 head branch に 1 件でも `mergedAt == null` があれば unsafe」を正本ルールにする。

### ユーザーシナリオとテスト（受け入れシナリオ）

- US1 (P0): 同じ head branch に `closed/unmerged` PR と `merged` PR が混在していても、Cleanup は warning 扱いにする。
- US2 (P0): 同じ head branch に `open` PR と `merged` PR が混在していても、Cleanup は warning 扱いにする。
- US3 (P0): 同じ head branch の全 PR が merged の場合に限り、post-merge commits 判定へ進む。
- US4 (P1): latest PR 表示が merged PR を示していても、branch safety は別ロジックで warning を維持する。

### 機能要件

- FR-001: branch safety 判定は、同一 `headRefName` に紐づく全 PR を対象に集約しなければならない。
- FR-002: `mergedAt == null` の PR が 1 件でも存在する場合、その branch は unsafe と判定しなければならない。
- FR-003: latest PR 表示用の latest 1 件選択と、branch safety 判定ロジックを分離しなければならない。
- FR-004: post-merge commit 判定は、対象 branch の全 PR が merged の場合にのみ実行しなければならない。
- FR-005: Cleanup の effective safety は `merged` のみ safe、`open` / `closed` / `none` は warning としなければならない。
- FR-006: `gwt-pr-check` / `gwt-pr` と gwt 本体は、未マージ判定において `mergedAt == null` を同じ意味で扱わなければならない。

### 非機能要件

- NFR-001: 判定は deterministic であり、同じ PR 集合に対して常に同じ結果を返すこと。
- NFR-002: mixed PR history (`closed/open + merged`) の回帰を Rust と frontend の自動テストで検知できること。
- NFR-003: latest PR 表示機能の既存挙動を不要に壊さないこと。

### 成功基準

- SC-001: `closed/unmerged + merged` の branch が warning になる。
- SC-002: `open + merged` の branch が warning になる。
- SC-003: all merged の branch のみ post-merge 判定へ進む。
- SC-004: latest PR 表示が merged でも Cleanup safety が safe に誤判定されない。
