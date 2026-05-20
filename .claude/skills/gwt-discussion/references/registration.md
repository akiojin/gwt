# Registration Reference

Detailed logic for the SPEC creation/update phase of gwt-discussion.

> **Preferred path (SPEC-2784):** When this discussion produces a *new*
> SPEC owner, hand the title + body off to the `gwt-register-spec`
> sub-skill instead of running `gwtd issue spec create` directly. The
> sub-skill enforces the 2-step `create` → `--edit spec` → roundtrip
> verify flow that this reference once documented manually, so the
> section-marker trap (an empty spec section after a bare `create -f`)
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

SPEC の作成・更新は `gwtd issue spec` CLI で行う。
すべての要約・タイトル・更新説明は current user's language で記述する。

**重要:** `gwtd issue spec create -f <body>` は body file が
`<!-- artifact:spec BEGIN/END -->` マーカーを含まないと `extract_sections()`
が空を返し、spec section が空のまま Issue が作成される (SPEC #2780 で発生)。
manual 経路では必ず以下の 2 段階で実行する:

1. `gwtd issue spec create --title "SPEC: <title>" -f <empty-or-stub>` で器を作成
2. `gwtd issue spec <n> --edit spec -f <full-body>` で content を投入
   (`--edit` は section マーカーを自動付与する)
3. `gwtd issue spec <n> --section spec | head -5` で非空を必ず確認

### コマンド

```bash
# SPEC 一覧
gwtd issue spec list

# SPEC 作成（構造化 JSON 推奨）
gwtd issue spec create --help
gwtd issue spec create --json --title "SPEC: <説明> — <サブタイトル>" \
  -f <spec.json>

# SPEC 作成（既存 Markdown 断片から直接作る互換パス）
gwtd issue spec create --title "SPEC: <説明> — <サブタイトル>" \
  -f <spec.md>

# SPEC セクション読み取り
gwtd issue spec <Issue番号>
gwtd issue spec <Issue番号> --section spec

# SPEC セクション更新
gwtd issue spec <Issue番号> --edit spec -f <file>
```

注意:

- `gwtd issue spec create --help` をフォーマット、JSON スキーマ、入力例の唯一の正とする。
- `spec create -f` はファイル内に `<!-- artifact:spec BEGIN/END -->` マーカーを期待する。
- マーカーなしの場合は `spec create` でタイトルだけ作成し、`--edit spec -f` または
  `--edit spec --json` で内容を投入する。

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
