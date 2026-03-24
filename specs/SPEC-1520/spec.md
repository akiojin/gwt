### 背景

Project Index の files/index/recovery 仕様が複数の Issue に分散していた。

- `#1427` は ChromaDB コレクション名の一時的な refactor 仕様
- `#1395` は project-scoped な ChromaDB 分離を含むが、Project Index files 全体仕様ではない
- `#1519` では persisted Chroma DB が壊れた状態で `files` collection の `count()` が `SIGSEGV` する failure mode が確認された

一方、Issue/Spec search の canonical ownership は `#1643` にあり、Issue semantic search の元データと更新ポリシーは `#1714` の local issue cache を参照する。本仕様は files/index/recovery のみを扱う。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: Files index を通常利用できる** [P0]
- 前提: worktree を gwt で開いている
- 操作: Files index を build/search/status する
- 期待: worktree 配下の `.gwt/index` を使って検索できる

**US-2: 既存 persisted DB が壊れていても回復する** [P0]
- 前提: worktree 配下の `.gwt/index` が Chroma runtime で `SIGSEGV` を起こす状態
- 操作: files の index/search/status を実行する
- 期待: crashing DB を quarantine し、files index を rebuild してから、元の操作を 1 回だけ retry する

**US-3: Project Index は files 専用として理解できる** [P1]
- 前提: implementer が canonical spec を読む
- 操作: `Project Index` の仕様を確認する
- 期待: Issue search の ownership は `#1643`、cache/linkage は `#1714` であることが分かる

### 機能要件

**FR-001**: Project Index の DB は開いた project/worktree root ごとに `project_root/.gwt/index` を使用しなければならない
**FR-002**: Files index は `index` / `search` / `status` を提供しなければならない
**FR-003**: shared Chroma DB が `SIGSEGV` 等で crash した場合、backend は `.gwt/index` を quarantine して files index を rebuild し、元の操作を 1 回だけ retry しなければならない
**FR-004**: `#1427` の恒久要件（files collection 名）と `#1519` の persisted DB recovery は、この files/index spec に集約されなければならない
**FR-005**: Issue/Spec search の canonical ownership は `#1643` を参照しなければならない
**FR-006**: Issue semantic search の元データと更新ポリシーは `#1714` を参照しなければならない

### 非機能要件

**NFR-001**: persisted DB corruption があっても、optional な Project Index failure がアプリ全体の利用継続性を破壊しない
**NFR-002**: Files index / recovery は自動テストまたは manual harness で回帰検知できる
**NFR-003**: Files index canonical spec だけを読めば files/index/recovery の責務が分かる

### 成功基準

1. Files index の `index` / `search` / `status` が worktree ごとに成立する
2. crashing persisted DB に対する recovery harness が GREEN になる
3. `#1520` を読んだ implementer が Issue search を正本として誤読しない
4. `#1643` と `#1714` への責務参照が明確である
