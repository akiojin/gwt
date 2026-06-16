# Registration Reference

Detailed logic for the SPEC creation/update phase of gwt-discussion.

## Prerequisites

Phase 3 runs only when Phase 1 routing produced `NEW-SPEC` and Phase 2 domain
discovery confirmed the scope is right for a SPEC.

## SPEC creation

SPEC の作成・更新は JSON operation `issue.spec.*` で行う。
すべての要約・タイトル・更新説明は current user's language で記述する。

### コマンド

```bash
# SPEC 一覧
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.list","params":{}}
JSON

# SPEC 作成（構造化 JSON 推奨）
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: <説明> — <サブタイトル>","structured":true,"body":{"spec":"<structured spec body>"}}}
JSON

# SPEC 作成（既存 Markdown 断片から直接作る互換パス）
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: <説明> — <サブタイトル>","body":"<spec markdown>"}}
JSON

# SPEC セクション読み取り
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.read","params":{"number":123}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":123,"section":"spec"}}
JSON

# SPEC セクション更新
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.edit","params":{"number":123,"section":"spec","body":"<full body>"}}
JSON
```

注意:

- JSON envelope schema in `json_envelope.rs` をフォーマット、JSON スキーマ、入力例の正とする。
- `issue.spec.create` はファイル内に `<!-- artifact:spec BEGIN/END -->` マーカーを期待する場合がある。
- マーカーなしの場合は `issue.spec.create` でタイトルだけ作成し、
  JSON operation `issue.spec.edit` で内容を投入する。

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
