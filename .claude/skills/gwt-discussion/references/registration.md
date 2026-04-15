# Registration Reference

Detailed logic for the SPEC creation/update phase of gwt-discussion.

## Prerequisites

Phase 3 runs only when Phase 1 routing produced `NEW-SPEC` and Phase 2 domain
discovery confirmed the scope is right for a SPEC.

## SPEC ID allocation

SPEC ID は GitHub Issue 番号をそのまま使用する。`gwt issue spec create` で
Issue を作成すると番号が自動割り当てされる。

既存 SPEC 一覧の確認:

```bash
gwt issue spec list
```

## Title convention

タイトルは `SPEC-<Issue#>:` プレフィックスを使用する:

```text
SPEC-<Issue#>: <現在のユーザー言語での簡潔な説明> — <サブタイトル>
```

- `SPEC-<Issue#>:` プレフィックスは Issue 作成後に確定する。作成時は
  `SPEC:` を仮プレフィックスとし、作成後にタイトルを更新する。
- サブタイトルは主要な機能領域を中黒 (・) 区切りで列挙する。

## SPEC creation

```bash
gwt issue spec create \
  --title "SPEC: <説明> — <サブタイトル>" \
  -f /tmp/spec.md \
  --label gwt-spec
```

作成後、返された Issue 番号でタイトルを更新する。

## Seeding spec.md

SPEC 本文は intake memo と domain model summary から作成する。

### Template

現在のユーザー言語で記述する。ドメイン固有名詞は英語のまま使用してよい。

```markdown
# SPEC-<Issue#>: <タイトル> — <サブタイトル>

## 状態

| 項目 | 値 |
|------|-----|
| ステータス | planned |
| 親 SPEC | #<親Issue番号> (SPEC-<親Issue番号>) |

## 背景

<intake memo の Request + Why now から>

## ユビキタス言語

<domain model の用語定義から>

| 用語 | 定義 |
|------|------|
| <用語> | <定義> |

## ユーザーストーリー

### US-1: <タイトル> (P0) -- planned

<アクター>として、<目標>をしたい。<理由>のために。

**受け入れシナリオ**

1. Given <前提条件>, when <操作>, then <期待結果>.

## エッジケース

- <エッジケースの説明>

## 機能要件

- **FR-001**: <要件>

## 非機能要件

- **NFR-001**: <要件>

## 成功基準

- **SC-001**: <測定可能な基準>
```

### Format rules

- US タイトル: `### US-N: <タイトル> (PN) -- <Status>`
  - Priority: P0 (必須) / P1 (重要) / P2 (将来)
  - Status: done / in-progress / planned
- FR/NFR/SC: 三桁ゼロ埋め (FR-001, NFR-001, SC-001)
- サブ要件: `FR-005a`, `FR-005b` 形式
- エッジケース: 番号なし箇条書き
- コード例: 言語指定付きコードブロック
- 受け入れシナリオ: 番号付き Given-When-Then 形式
- 相互参照: `#<Issue番号> (SPEC-<Issue番号>)` 形式
- 更新履歴: `## YYYY-MM-DD 更新: <内容>` セクション

### Rules for seeding

- Populate from the intake memo and domain model. Do not invent requirements.
- Use `[NEEDS CLARIFICATION: <question>]` for unknowns instead of guessing.
- Include the ユビキタス言語 section from Phase 2.
- Map user stories to the entities and BCs identified in Phase 2.
- Do not create plan.md or tasks.md at this phase.

### Upload

```bash
cat <<'SPEC_EOF' > /tmp/spec.md
<spec content>
SPEC_EOF

gwt issue spec create \
  --title "SPEC: <説明>" \
  -f /tmp/spec.md \
  --label gwt-spec
```

## Post-registration

After creating the SPEC and seeding spec.md, proceed directly to Phase 4
(Clarification). Do not stop unless the user explicitly requested register-only.

## SPEC storage structure (for reference)

SPEC は GitHub Issue の body 内に `<!-- artifact:NAME BEGIN/END -->` マーカーで
格納される。大きなセクションは Issue comment に分割される。

| セクション | 格納先 | 作成タイミング |
|-----------|--------|--------------|
| spec | Issue body (default) or comment | Phase 3 (Registration) |
| plan | Issue comment | gwt-plan-spec |
| tasks | Issue comment | gwt-plan-spec |

ローカルキャッシュ: `~/.gwt/cache/issues/<Issue番号>/`

```text
~/.gwt/cache/issues/<Issue番号>/
  body.md            # Issue body raw text
  spec.md            # parsed spec section content (no markers)
  plan.md            # parsed plan section content
  tasks.md           # parsed tasks section content
```
