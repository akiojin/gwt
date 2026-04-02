> **Canonical Boundary**: `SPEC-1777` は management workspace における local SPEC viewer と launch entry の正本である。SPEC workflow owner は `SPEC-1579` / `SPEC-1787`、parent navigation は `SPEC-1776` が担当する。

# SPECs タブ — 一覧・詳細・検索・起動導線

## Background

`SPECs` タブは local `specs/SPEC-*` artifact の viewer であり、`Issues` タブとは独立して human-facing な閲覧・検索・起動導線を提供する。rebuilt TUI では `SPECs` タブも management workspace の first-class tab として残し、branch-first UX の中で `launch from SPEC` を first-class に維持する。

## User Stories

### US-1: local SPEC 一覧を見たい

### US-2: artifact section を切り替えて詳細を見たい

### US-3: SPEC を検索したい

### US-4: SPEC から agent launch へ入りたい

## Acceptance Scenarios

1. `SPECs` タブで local SPEC 一覧が表示される
2. detail view で `spec / plan / tasks / research / data-model / quickstart / checklists / contracts` を切り替えられる
3. 検索で title や ID に一致する SPEC を絞り込める
4. 選択した SPEC から launch entry に入れる
5. `Issues` タブとは state と責務を共有しない

## Functional Requirements

- FR-001: local SPEC 一覧を構築する
- FR-002: artifact section 切替 detail を提供する
- FR-003: title / ID ベースの検索を提供する
- FR-004: selected SPEC から launch entry へ進める
- FR-005: `Issues` タブと責務分離を維持する
- FR-006: workflow owner は `SPEC-1579` / `SPEC-1787` であり、本タブは viewer / entry role のみを持つ

## Success Criteria

- SC-001: `SPECs` タブが rebuilt management workspace の first-class tab として成立する
- SC-002: detail, search, launch entry が parent UX と矛盾しない
