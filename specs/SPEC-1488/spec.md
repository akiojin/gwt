### 背景

Codex セッション中に Issue-first の SPEC を作成・更新する必要があったが、repo 内に存在する `gwt-issue-spec-ops` スキルが available skills に露出しておらず、Codex がそのスキルを選択・利用できなかった。

一方で、repo には以下の根拠が存在する。

- [gwt-issue-spec-ops skill](https://github.com/akiojin/gwt/blob/develop/plugins/gwt/skills/gwt-issue-spec-ops/SKILL.md)
- [gwt-issue-spec-ops command](https://github.com/akiojin/gwt/blob/develop/plugins/gwt/commands/gwt-issue-spec-ops.md)
- [managed skill registration](https://github.com/akiojin/gwt/blob/develop/crates/gwt-core/src/config/skill_registration.rs)
- [Issue-first spec運用ルール](https://github.com/akiojin/gwt/blob/develop/CLAUDE.md)

`gwt-issue-spec-ops` は `gwt-spec` Issue 上で Spec / Plan / Tasks / TDD を管理するための repo 固有スキルとして設計されている。それにもかかわらず、Codex がそのスキルを利用できず、汎用スキルや手動 `gh` 操作へフォールバックしてしまう。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: SPEC作成要求で repo 固有スキルが選ばれる** [P0]

- 前提: gwt 配下のプロジェクトを gwt 経由で Codex セッションとして起動している
- 操作: ユーザーが「SPECを作成して」「Issue登録して」「TDD込みで仕様化して」と依頼する
- 期待: `gwt-issue-spec-ops` が available skills に現れ、Codex がまずそのスキルを利用する

**US-2: available skills 一覧から SPEC登録用途が判別できる** [P1]

- 前提: gwt プロジェクトで Codex セッションを開始する
- 操作: AGENTS / skill list を確認する
- 期待: `gwt-issue-spec-ops` が表示され、Issue-first spec 用途が判別できる

**US-3: `gwt-spec` Issue 作成テンプレートが一貫して使われる** [P1]

- 前提: 新規仕様または仕様更新が必要
- 操作: `gwt-issue-spec-ops` を使って Issue を作成/更新する
- 期待: `GWT_SPEC_ID`、`Spec / Plan / Tasks / TDD` を含む統一テンプレートが使われる

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | gwt プロジェクトで Codex セッションを開始したとき、`gwt-issue-spec-ops` を available skills に含めなければならない |
| FR-002 | `gwt-issue-spec-ops` の metadata / description は SPEC 作成・Issue 登録・TDD 設計用途を識別できる文言を含まなければならない |
| FR-003 | repo に managed skill として登録されている `gwt-issue-spec-ops` は、セッション側の skills 一覧にも一貫して反映されなければならない |
| FR-004 | SPEC / Issue 作成要求時は、汎用 spec スキルより `gwt-issue-spec-ops` を優先的に選択できる状態でなければならない |
| FR-005 | `gwt-issue-spec-ops` で作成される `gwt-spec` Issue は repo の Issue-first テンプレート構造と一致しなければならない |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 既存の他 managed skills の露出挙動を壊さない |
| NFR-002 | skill discoverability と skill selection 優先順位を自動テストで検知できる |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | gwt プロジェクトの Codex セッションで `gwt-issue-spec-ops` が available skills 一覧に表示される |
| SC-002 | SPEC 作成要求に対して `gwt-issue-spec-ops` 利用が選ばれる再現手順が確認できる |
| SC-003 | `gwt-spec` Issue のテンプレート構造が repo 運用と一致する |
