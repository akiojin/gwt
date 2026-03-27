### 背景

Codex セッション中に SPEC を探索する必要があったが、repo 内に存在する `gwt-project-search` スキルが available skills に露出しておらず、Codex がそのスキルを選択・利用できなかった。

一方で、repo には以下の根拠が存在する。

- [gwt-project-search skill](https://github.com/akiojin/gwt/blob/develop/plugins/gwt/skills/gwt-project-search/SKILL.md)
- [gwt-project-search command](https://github.com/akiojin/gwt/blob/develop/plugins/gwt/commands/gwt-project-search.md)
- [managed skill registration](https://github.com/akiojin/gwt/blob/develop/crates/gwt-core/src/config/skill_registration.rs)
- [Issue-first spec運用ルール](https://github.com/akiojin/gwt/blob/develop/CLAUDE.md)
- [spec一覧導線](https://github.com/akiojin/gwt/blob/develop/specs/specs.md)

`gwt-project-search` は SPEC/Issue を semantic search するための専用スキルとして設計されているにもかかわらず、Codex から見える利用可能スキル一覧に現れない。その結果、仕様探索で repo 固有ワークフローではなく手動検索へフォールバックしてしまう。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: SPEC探索時に専用スキルが選ばれる** [P0]

- 前提: gwt 配下のプロジェクトを gwt 経由で Codex セッションとして起動している
- 操作: ユーザーが「SPECを検索して」「関連仕様を探して」と依頼する
- 期待: `gwt-project-search` が available skills に現れ、Codex がまずそのスキルを利用する

**US-2: available skills 一覧から SPEC検索用途が判別できる** [P1]

- 前提: gwt プロジェクトで Codex セッションを開始する
- 操作: AGENTS / skill list を確認する
- 期待: `gwt-project-search` が表示され、説明文から SPEC/Issue 検索用途が判別できる

**US-3: スキル説明どおりに最短で復旧できる** [P1]

- 前提: index が未作成または古い
- 操作: `gwt-project-search` を利用する
- 期待: スキルが正しい復旧手順を案内し、必要な引数不足や誤案内がない

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | gwt プロジェクトで Codex セッションを開始したとき、`gwt-project-search` を available skills に含めなければならない |
| FR-002 | `gwt-project-search` の metadata / description は SPEC 検索用途を識別できる文言を含まなければならない |
| FR-003 | repo に managed skill として登録されている `gwt-project-search` は、セッション側の skills 一覧にも一貫して反映されなければならない |
| FR-004 | index 不在時の案内は、実装されている CLI 仕様と一致しなければならない |
| FR-005 | SPEC 検索要求時は、汎用検索より `gwt-project-search` を優先的に選択できる状態でなければならない |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 既存の他 managed skills の露出挙動を壊さない |
| NFR-002 | skill discoverability の回帰を自動テストで検知できる |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | gwt プロジェクトの Codex セッションで `gwt-project-search` が available skills 一覧に表示される |
| SC-002 | SPEC 検索要求に対して `gwt-project-search` 利用が選ばれる再現手順が確認できる |
| SC-003 | スキル説明どおりに index-issues / search-issues が実行できる |
