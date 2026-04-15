# Registration Reference

Detailed logic for the SPEC creation/update phase of gwt-discussion.

## Prerequisites

Phase 3 runs only when Phase 1 routing produced `NEW-SPEC` and Phase 2 domain
discovery confirmed the scope is right for a SPEC.

## SPEC creation

SPEC の作成・更新は `gwt issue spec` CLI で行う。
フォーマット、JSON スキーマ、入力例は `gwt issue spec create --help` を正として参照すること。

### 現行コマンド

```bash
# SPEC 一覧
gwt issue spec list

# SPEC 作成（構造化 JSON 推奨）
gwt issue spec create --help
gwt issue spec create --json --title "SPEC: <説明> — <サブタイトル>" \
  -f <spec.json>

# SPEC 作成（既存 Markdown 断片から直接作る互換パス）
gwt issue spec create --title "SPEC: <説明> — <サブタイトル>" \
  -f <spec.md>

# SPEC セクション読み取り
gwt issue spec <Issue番号>
gwt issue spec <Issue番号> --section spec

# SPEC セクション更新
gwt issue spec <Issue番号> --edit spec -f <file>
```

### Title convention

```text
SPEC-<Issue#>: <現在のユーザー言語での簡潔な説明> — <サブタイトル>
```

`SPEC-<Issue#>:` プレフィックスは Issue 作成後に確定する。

### Seeding rules

- Populate from the intake memo and domain model. Do not invent requirements.
- Use `[NEEDS CLARIFICATION: <question>]` for unknowns instead of guessing.
- Include ユビキタス言語 from Phase 2.
- Map user stories to the entities and BCs identified in Phase 2.
- Do not create plan or tasks at this phase.

## Post-registration

After creating the SPEC, proceed directly to Phase 4 (Clarification). Do not
stop unless the user explicitly requested register-only.

## SPEC storage

SPEC は GitHub Issue の body 内に `<!-- artifact:NAME BEGIN/END -->` マーカーで
格納される。大きなセクションは Issue comment に分割される。

ローカルキャッシュ: `~/.gwt/cache/issues/<Issue番号>/`
