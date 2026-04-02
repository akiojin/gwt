> **Canonical Boundary**: `SPEC-1782` は branch-scoped Quick Start fast-launch contract の正本である。parent branch entry behavior は `SPEC-1776`、workflow owner は `SPEC-1579` / `SPEC-1787` が担当する。

# Quick Start — ブランチ単位の高速起動

## Background

rebuilt TUI では `1ブランチ = Nセッション` を許可するため、Quick Start はもはや branch `Enter` の唯一の結果ではない。それでも、branch ごとの直近設定を使った resume / new-session fast path は引き続き重要である。本 SPEC は Quick Start を branch-scoped fast-launch contract として定義し、session selector と共存させる。

## User Stories

### US-1: branch ごとの前回設定で素早く再開したい

### US-2: active session があるブランチでも追加起動したい

### US-3: 必要ならフル Wizard へ落ちたい

## Acceptance Scenarios

1. branch に active session がなく、resume 可能な履歴がある場合、Quick Start fast path を提示できる
2. branch に active session が複数ある場合、selector から `追加起動` を選ぶと Quick Start fast path へ進める
3. Resume は保存済み session_id を使って起動する
4. Start New は保存済み設定を使って新規 session を追加する
5. いつでも Full Wizard へフォールバックできる

## Functional Requirements

- FR-001: Quick Start は branch-scoped fast-launch contract を提供する
- FR-002: Quick Start は `1ブランチ = Nセッション` と両立しなければならない
- FR-003: branch `Enter` の selector から `追加起動` fast path として呼び出せなければならない
- FR-004: Resume は session_id を使って起動する
- FR-005: Start New は保存済み設定を使って新規 session を追加する
- FR-006: Full Wizard へのフォールバックを常に提供する

## Success Criteria

- SC-001: Quick Start が branch-first parent UX と矛盾しない
- SC-002: active session の有無にかかわらず、resume / add session / full wizard の関係が一貫する
