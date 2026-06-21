# Registration Reference

Detailed logic for the SPEC creation/update phase of gwt-discussion.

> **Preferred path (SPEC-2784):** When this discussion produces a *new*
> SPEC owner, hand the title + body off to the `gwt-register-spec`
> sub-skill instead of running `issue.spec.create` directly. The
> sub-skill enforces the 2-step `create` → `issue.spec.edit` → roundtrip
> verify flow that this reference once documented manually, so the
> section-marker trap (an empty spec section after a bare create-body call)
> cannot occur. Add `Register Spec` to the Action Bundle and let
> gwt-register-spec own the registration.
>
> The rest of this document remains as the recovery / manual path for
> when the sub-skill is unavailable, or when *updating* an existing SPEC
> rather than creating a new one.

## Prerequisites

Phase 3 runs only when Phase 1 routing produced `NEW-SPEC` and Phase 2 domain
discovery confirmed the scope is right for a SPEC.

## SPEC creation (manual / recovery path)

SPEC の作成・更新は JSON operation `issue.spec.*` で行う。
すべての要約・タイトル・更新説明は current user's language で記述する。

**重要:** create-body transport は body file が
`<!-- artifact:spec BEGIN/END -->` マーカーを含まないと `extract_sections()`
が空を返し、spec section が空のまま Issue が作成される (SPEC #2780 で発生)。
manual 経路では必ず以下の 2 段階で実行する:

1. JSON operation `issue.spec.create` で器を作成
2. JSON operation `issue.spec.edit` で content を投入
   (`issue.spec.edit` は section マーカーを自動付与する)
3. JSON operation `issue.spec.section` で非空を必ず確認

### コマンド

```bash
# SPEC 一覧
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.list","params":{}}
JSON

# SPEC 作成（構造化 JSON 推奨）
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: <説明> — <サブタイトル>","structured":true,"body":{"spec":"<structured spec body>"}}}
JSON

# SPEC 作成（既存 Markdown 断片から直接作る互換パス）
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: <説明> — <サブタイトル>","body":"<spec markdown>"}}
JSON

# SPEC セクション読み取り
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.read","params":{"number":123}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":123,"section":"spec"}}
JSON

# SPEC セクション更新
"$GWT_BIN" <<'JSON'
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
